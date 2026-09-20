//! Pure, bounded text observations for admitted edit sessions.
//!
//! This module deliberately has no filesystem or journal dependency.  Its caller
//! is responsible for authorizing and reading the immutable preimage before it is
//! passed here.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_TEXT_BYTES: usize = 1_048_576;
const MAX_EXCERPT_BYTES: usize = 2_048;
const MAX_FIND_EXCERPT_BYTES: usize = 256;
const MAX_FIND_MATCHES: usize = 8;
const MAX_OBSERVATION_JSON_BYTES: usize = 16_384;

/// A single model-requested, pure text observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextReadOperation {
    ReadTaskText {
        path: String,
        start_line: u64,
        line_count: u64,
    },
    FindTaskText {
        path: String,
        literal: String,
    },
}

impl TextReadOperation {
    pub fn path(&self) -> &str {
        match self {
            Self::ReadTaskText { path, .. } | Self::FindTaskText { path, .. } => path,
        }
    }

    /// Validates operation-local request bounds. Path authorization belongs at the
    /// filesystem boundary, where it can be checked against the current grant.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::ReadTaskText {
                start_line,
                line_count,
                ..
            } => {
                ensure!(
                    *start_line > 0,
                    "read_task_text start_line must be positive"
                );
                ensure!(
                    (1..=128).contains(line_count),
                    "read_task_text line_count must be between 1 and 128"
                );
            }
            Self::FindTaskText { literal, .. } => {
                ensure!(
                    !literal.is_empty(),
                    "find_task_text literal must not be empty"
                );
                ensure!(
                    literal.len() <= MAX_FIND_EXCERPT_BYTES,
                    "find_task_text literal exceeds 256 UTF-8 bytes"
                );
            }
        }
        Ok(())
    }
}

/// The newline convention observed in the complete immutable preimage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewlineStyle {
    Lf,
    Crlf,
    Mixed,
    None,
}

/// Exact newline facts. Lone carriage returns are content, rather than lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewlineMetadata {
    pub style: NewlineStyle,
    pub lone_cr_count: u64,
    pub final_lf: bool,
}

/// A byte-exact excerpt. Byte ranges are half-open offsets into the complete file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextExcerpt {
    pub exact_utf8: String,
    pub hash: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u64,
    pub end_line: u64,
    pub partial_last_line: bool,
    pub truncated: bool,
}

/// A complete literal-match location, even when the associated excerpt is short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextMatch {
    pub start_byte: u64,
    pub end_byte: u64,
}

/// A bounded read/find observation over one complete UTF-8 preimage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextReadResult {
    pub file_hash: String,
    pub excerpts: Vec<TextExcerpt>,
    pub matches: Vec<TextMatch>,
    pub matches_truncated: bool,
    pub truncated: bool,
    pub newline: NewlineMetadata,
}

impl TextReadResult {
    /// Returned UTF-8 byte count. Repeated text in overlapping excerpts counts
    /// repeatedly, because it consumes context repeatedly.
    pub fn text_bytes(&self) -> usize {
        self.excerpts.iter().fold(0usize, |total, excerpt| {
            total.saturating_add(excerpt.exact_utf8.len())
        })
    }
}

/// Produces one exact, bounded observation without filesystem or journal I/O.
pub fn observe_text(
    bytes: &[u8],
    operation: &TextReadOperation,
    allowance: usize,
) -> Result<TextReadResult> {
    operation.validate()?;
    ensure!(allowance > 0, "text observation allowance must be positive");
    ensure!(
        bytes.len() <= MAX_TEXT_BYTES,
        "text observation exceeds the 1 MiB preimage bound"
    );
    let text = std::str::from_utf8(bytes)
        .map_err(|error| anyhow::anyhow!("text observation requires valid UTF-8: {error}"))?;
    let lines = line_spans(bytes);
    let newline = newline_metadata(bytes);
    let byte_budget = allowance.min(MAX_EXCERPT_BYTES);

    let mut result = match operation {
        TextReadOperation::ReadTaskText {
            start_line,
            line_count,
            ..
        } => observe_lines(bytes, text, &lines, *start_line, *line_count, byte_budget)?,
        TextReadOperation::FindTaskText { literal, .. } => {
            observe_literal(bytes, text, &lines, literal.as_bytes(), byte_budget)?
        }
    };
    result.file_hash = hash(bytes);
    result.newline = newline;

    let serialized = serde_json::to_vec(&result)?;
    ensure!(
        serialized.len() <= MAX_OBSERVATION_JSON_BYTES,
        "text observation JSON exceeds 16 KiB"
    );
    Ok(result)
}

fn observe_lines(
    bytes: &[u8],
    text: &str,
    lines: &[(usize, usize)],
    start_line: u64,
    line_count: u64,
    budget: usize,
) -> Result<TextReadResult> {
    ensure!(
        !lines.is_empty(),
        "read_task_text start_line is out of range"
    );
    let total_lines =
        u64::try_from(lines.len()).map_err(|_| anyhow::anyhow!("line count is out of range"))?;
    ensure!(
        start_line <= total_lines,
        "read_task_text start_line is out of range"
    );

    let requested_last = start_line
        .checked_add(line_count - 1)
        .ok_or_else(|| anyhow::anyhow!("read_task_text line range is out of range"))?;
    let actual_last = requested_last.min(total_lines);
    let start_index = usize::try_from(start_line - 1)
        .map_err(|_| anyhow::anyhow!("read_task_text start_line is out of range"))?;
    let last_index = usize::try_from(actual_last - 1)
        .map_err(|_| anyhow::anyhow!("read_task_text line range is out of range"))?;
    let start = lines[start_index].0;
    let wanted_end = lines[last_index].1;
    let end = bounded_end(bytes, text, start, wanted_end, budget);
    let clipped_to_eof = requested_last > total_lines;
    let byte_truncated = end < wanted_end;
    let truncated = clipped_to_eof || byte_truncated;

    Ok(TextReadResult {
        file_hash: String::new(),
        excerpts: vec![excerpt(
            bytes,
            text,
            start,
            end,
            start_line,
            line_for_end(lines, end, start_line),
            partial_last_line(bytes, start, end, wanted_end),
            truncated,
        )],
        matches: Vec::new(),
        matches_truncated: false,
        truncated,
        newline: placeholder_newline(),
    })
}

fn observe_literal(
    bytes: &[u8],
    text: &str,
    lines: &[(usize, usize)],
    literal: &[u8],
    budget: usize,
) -> Result<TextReadResult> {
    let locations = literal_locations(bytes, literal);
    let matches_truncated = locations.len() > MAX_FIND_MATCHES;
    let retained = &locations[..locations.len().min(MAX_FIND_MATCHES)];
    let mut remaining = budget;
    let mut excerpts = Vec::with_capacity(retained.len());
    let mut any_excerpt_truncated = false;

    for &start in retained {
        let natural_end = start
            .saturating_add(MAX_FIND_EXCERPT_BYTES)
            .min(bytes.len());
        let permitted_end = start.saturating_add(remaining).min(natural_end);
        let end = bounded_end(bytes, text, start, permitted_end, remaining);
        let used = end - start;
        remaining -= used;
        let truncated = end < bytes.len();
        any_excerpt_truncated |= truncated;
        excerpts.push(excerpt(
            bytes,
            text,
            start,
            end,
            line_for_offset(lines, start),
            line_for_end(lines, end, line_for_offset(lines, start)),
            partial_last_line(bytes, start, end, bytes.len()),
            truncated,
        ));
    }

    Ok(TextReadResult {
        file_hash: String::new(),
        excerpts,
        matches: retained
            .iter()
            .map(|&start| TextMatch {
                start_byte: u64::try_from(start).expect("bounded file size fits u64"),
                end_byte: u64::try_from(start + literal.len()).expect("bounded file size fits u64"),
            })
            .collect(),
        matches_truncated,
        truncated: matches_truncated || any_excerpt_truncated,
        newline: placeholder_newline(),
    })
}

fn literal_locations(bytes: &[u8], literal: &[u8]) -> Vec<usize> {
    debug_assert!(!literal.is_empty());
    let mut locations = Vec::new();
    let mut offset = 0;
    while offset + literal.len() <= bytes.len() {
        if &bytes[offset..offset + literal.len()] == literal {
            locations.push(offset);
            // Eight locations are returned and the ninth is enough to record
            // truncation. Do not retain unbounded match metadata for a dense file.
            if locations.len() > MAX_FIND_MATCHES {
                break;
            }
        }
        offset += 1;
    }
    locations
}

fn line_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            lines.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < bytes.len() {
        lines.push((start, bytes.len()));
    }
    lines
}

fn newline_metadata(bytes: &[u8]) -> NewlineMetadata {
    let mut bare_lf = false;
    let mut crlf = false;
    let mut lone_cr_count = 0u64;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
            lone_cr_count = lone_cr_count.saturating_add(1);
        }
        if byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                crlf = true;
            } else {
                bare_lf = true;
            }
        }
    }
    let style = match (bare_lf, crlf) {
        (false, false) => NewlineStyle::None,
        (true, false) => NewlineStyle::Lf,
        (false, true) => NewlineStyle::Crlf,
        (true, true) => NewlineStyle::Mixed,
    };
    NewlineMetadata {
        style,
        lone_cr_count,
        final_lf: bytes.last() == Some(&b'\n'),
    }
}

fn bounded_end(bytes: &[u8], text: &str, start: usize, wanted_end: usize, budget: usize) -> usize {
    let target = wanted_end.min(start.saturating_add(budget));
    let mut end = target;
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    // Keeping a CR while dropping its LF would change a CRLF line ending.
    if end > start && end < bytes.len() && bytes[end - 1] == b'\r' && bytes[end] == b'\n' {
        end -= 1;
    }
    end
}

#[allow(clippy::too_many_arguments)]
fn excerpt(
    bytes: &[u8],
    text: &str,
    start: usize,
    end: usize,
    start_line: u64,
    end_line: u64,
    partial_last_line: bool,
    truncated: bool,
) -> TextExcerpt {
    debug_assert!(start <= end && end <= bytes.len());
    debug_assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
    let exact_utf8 = text[start..end].to_owned();
    TextExcerpt {
        hash: hash(exact_utf8.as_bytes()),
        exact_utf8,
        start_byte: u64::try_from(start).expect("bounded file size fits u64"),
        end_byte: u64::try_from(end).expect("bounded file size fits u64"),
        start_line,
        end_line,
        partial_last_line,
        truncated,
    }
}

fn line_for_offset(lines: &[(usize, usize)], offset: usize) -> u64 {
    if lines.is_empty() {
        return 0;
    }
    let index = lines.partition_point(|&(_, end)| end <= offset);
    u64::try_from(index.min(lines.len() - 1) + 1).expect("bounded line count fits u64")
}

fn line_for_end(lines: &[(usize, usize)], end: usize, fallback: u64) -> u64 {
    if lines.is_empty() || end == 0 {
        return fallback;
    }
    if end >= lines.last().expect("nonempty checked").1 {
        return u64::try_from(lines.len()).expect("bounded line count fits u64");
    }
    line_for_offset(lines, end.saturating_sub(1))
}

fn partial_last_line(bytes: &[u8], start: usize, end: usize, wanted_end: usize) -> bool {
    end < wanted_end && end > start && bytes[end - 1] != b'\n'
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn placeholder_newline() -> NewlineMetadata {
    NewlineMetadata {
        style: NewlineStyle::None,
        lone_cr_count: 0,
        final_lf: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(start_line: u64, line_count: u64) -> TextReadOperation {
        TextReadOperation::ReadTaskText {
            path: "src/lib.rs".into(),
            start_line,
            line_count,
        }
    }

    fn find(literal: &str) -> TextReadOperation {
        TextReadOperation::FindTaskText {
            path: "src/lib.rs".into(),
            literal: literal.into(),
        }
    }

    #[test]
    fn reads_lf_lines_without_a_trailing_phantom_line() {
        let result = observe_text(b"one\ntwo\n", &read(2, 8), 2_048).unwrap();
        assert_eq!(result.excerpts[0].exact_utf8, "two\n");
        assert_eq!(result.excerpts[0].start_byte, 4);
        assert_eq!(result.excerpts[0].end_byte, 8);
        assert_eq!(result.excerpts[0].start_line, 2);
        assert_eq!(result.excerpts[0].end_line, 2);
        assert!(result.truncated);
        assert_eq!(result.newline.style, NewlineStyle::Lf);
        assert!(result.newline.final_lf);
    }

    #[test]
    fn preserves_crlf_and_never_returns_half_of_one() {
        let result = observe_text(b"a\r\nb\r\n", &read(1, 1), 2).unwrap();
        assert_eq!(result.excerpts[0].exact_utf8, "a");
        assert_eq!(result.excerpts[0].end_byte, 1);
        assert!(result.excerpts[0].partial_last_line);
        assert!(result.excerpts[0].truncated);
        assert_eq!(result.newline.style, NewlineStyle::Crlf);
    }

    #[test]
    fn records_mixed_newlines_and_lone_carriage_returns_as_content() {
        let result = observe_text(b"a\r\nb\nc\rd", &read(1, 3), 2_048).unwrap();
        assert_eq!(result.excerpts[0].exact_utf8, "a\r\nb\nc\rd");
        assert_eq!(result.newline.style, NewlineStyle::Mixed);
        assert_eq!(result.newline.lone_cr_count, 1);
        assert!(!result.newline.final_lf);
    }

    #[test]
    fn bom_and_unicode_obey_scalar_boundaries() {
        let result = observe_text("\u{feff}🙂x\n".as_bytes(), &read(1, 1), 6).unwrap();
        assert_eq!(result.excerpts[0].exact_utf8, "\u{feff}");
        assert_eq!(result.excerpts[0].end_byte, 3);
        assert!(result.excerpts[0].partial_last_line);
        assert!(std::str::from_utf8(result.excerpts[0].exact_utf8.as_bytes()).is_ok());
    }

    #[test]
    fn long_line_is_bounded_and_marked_partial() {
        let data = "x".repeat(3_000);
        let result = observe_text(data.as_bytes(), &read(1, 1), 9_999).unwrap();
        assert_eq!(result.text_bytes(), 2_048);
        assert!(result.truncated);
        assert!(result.excerpts[0].partial_last_line);
    }

    #[test]
    fn empty_file_read_fails_but_find_succeeds_empty() {
        assert!(observe_text(b"", &read(1, 1), 1).is_err());
        let result = observe_text(b"", &find("needle"), 1).unwrap();
        assert!(result.excerpts.is_empty());
        assert!(result.matches.is_empty());
        assert_eq!(result.newline.style, NewlineStyle::None);
    }

    #[test]
    fn find_reports_overlaps_and_detects_the_ninth_match() {
        let result = observe_text(b"aaaaaaaaaa", &find("aa"), 2_048).unwrap();
        assert_eq!(result.matches.len(), 8);
        assert!(result.matches_truncated);
        assert!(result.truncated);
        assert_eq!(
            result
                .matches
                .iter()
                .map(|item| item.start_byte)
                .collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(result.text_bytes(), 52);
    }

    #[test]
    fn find_charges_duplicate_excerpt_bytes_against_the_shared_budget() {
        let result = observe_text(b"abababab", &find("ab"), 3).unwrap();
        assert_eq!(result.matches.len(), 4);
        assert_eq!(result.excerpts[0].exact_utf8, "aba");
        assert!(result.excerpts[1].exact_utf8.is_empty());
        assert_eq!(result.text_bytes(), 3);
    }

    #[test]
    fn clipping_to_eof_and_zero_scalar_room_are_recorded_without_panicking() {
        let clipped = observe_text(b"a\nb", &read(2, 128), 2_048).unwrap();
        assert!(clipped.truncated);
        assert!(!clipped.excerpts[0].partial_last_line);

        let no_scalar = observe_text("🙂".as_bytes(), &read(1, 1), 1).unwrap();
        assert!(no_scalar.excerpts[0].exact_utf8.is_empty());
        assert_eq!(no_scalar.excerpts[0].start_byte, 0);
        assert!(no_scalar.truncated);
    }

    #[test]
    fn validates_requests_and_inputs_without_panicking() {
        assert!(read(0, 1).validate().is_err());
        assert!(read(1, 0).validate().is_err());
        assert!(read(1, 129).validate().is_err());
        assert!(find("").validate().is_err());
        assert!(find(&"x".repeat(257)).validate().is_err());
        assert!(observe_text(b"x", &read(u64::MAX, 1), 1).is_err());
        assert!(observe_text(b"x", &read(1, 1), 0).is_err());
        assert!(observe_text(&[0xff], &read(1, 1), 1).is_err());
        assert!(observe_text(&vec![b'x'; MAX_TEXT_BYTES + 1], &read(1, 1), 1).is_err());
    }

    #[test]
    fn operation_wire_shape_rejects_unknown_fields() {
        let operation: TextReadOperation = serde_json::from_str(
            r#"{"tool":"read_task_text","path":"a","start_line":1,"line_count":1}"#,
        )
        .unwrap();
        assert_eq!(operation.path(), "a");
        assert!(
            serde_json::from_str::<TextReadOperation>(
                r#"{"tool":"find_task_text","path":"a","literal":"x","extra":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn result_json_remains_within_the_observation_cap() {
        let data = std::iter::repeat_n('\u{0001}', 2_048).collect::<String>();
        let result = observe_text(data.as_bytes(), &read(1, 1), 2_048).unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_OBSERVATION_JSON_BYTES);
    }
}
