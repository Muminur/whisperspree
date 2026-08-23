//! Word-error-rate helpers for deterministic ASR golden tests (PRD §17.3).

use unicode_normalization::UnicodeNormalization;

/// Normalizes text exactly as PRD §17.3 requires: NFC, lowercase, punctuation
/// removal, and whitespace collapse.
pub fn normalize(text: &str) -> String {
    text.nfc()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns word-level Levenshtein distance divided by the reference length.
/// An empty reference has zero WER only for an empty hypothesis; otherwise its
/// error rate is unbounded and represented as positive infinity.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let reference = words(reference);
    let hypothesis = words(hypothesis);
    if reference.is_empty() {
        return if hypothesis.is_empty() {
            0.0
        } else {
            f64::INFINITY
        };
    }
    levenshtein_matrix(&reference, &hypothesis)[reference.len()][hypothesis.len()] as f64
        / reference.len() as f64
}

/// Produces a compact, line-oriented alignment intended for golden-test
/// failures. A leading space is a match, `-` a missing reference word, and `+`
/// an unexpected hypothesis word.
pub fn alignment_diff(reference: &str, hypothesis: &str) -> String {
    let reference = words(reference);
    let hypothesis = words(hypothesis);
    let matrix = levenshtein_matrix(&reference, &hypothesis);
    let mut operations = Vec::new();
    let (mut row, mut column) = (reference.len(), hypothesis.len());

    while row > 0 || column > 0 {
        if row > 0
            && column > 0
            && reference[row - 1] == hypothesis[column - 1]
            && matrix[row][column] == matrix[row - 1][column - 1]
        {
            operations.push(format!("  {}", reference[row - 1]));
            row -= 1;
            column -= 1;
        } else if row > 0 && column > 0 && matrix[row][column] == matrix[row - 1][column - 1] + 1 {
            operations.push(format!("+ {}", hypothesis[column - 1]));
            operations.push(format!("- {}", reference[row - 1]));
            row -= 1;
            column -= 1;
        } else if row > 0 && matrix[row][column] == matrix[row - 1][column] + 1 {
            operations.push(format!("- {}", reference[row - 1]));
            row -= 1;
        } else {
            debug_assert!(column > 0);
            operations.push(format!("+ {}", hypothesis[column - 1]));
            column -= 1;
        }
    }

    operations.reverse();
    operations.join("\n")
}

fn words(text: &str) -> Vec<String> {
    normalize(text)
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect()
}

fn levenshtein_matrix(reference: &[String], hypothesis: &[String]) -> Vec<Vec<usize>> {
    let mut matrix = vec![vec![0; hypothesis.len() + 1]; reference.len() + 1];
    for (row, entry) in matrix.iter_mut().enumerate() {
        entry[0] = row;
    }
    for (column, entry) in matrix[0].iter_mut().enumerate() {
        *entry = column;
    }
    for row in 1..=reference.len() {
        for column in 1..=hypothesis.len() {
            matrix[row][column] = if reference[row - 1] == hypothesis[column - 1] {
                matrix[row - 1][column - 1]
            } else {
                1 + matrix[row - 1][column - 1]
                    .min(matrix[row - 1][column])
                    .min(matrix[row][column - 1])
            };
        }
    }
    matrix
}

#[cfg(test)]
mod tests {
    use super::{alignment_diff, normalize, word_error_rate};

    #[test]
    fn wer_normalizes_case_punctuation_unicode_and_whitespace() {
        assert_eq!(normalize("  HÉLLO,\nworld!  "), "héllo world");
        assert_eq!(normalize("cafe\u{301}"), "café");
        assert_eq!(word_error_rate("Hello, world!", "hello world"), 0.0);
    }

    #[test]
    fn wer_counts_word_level_substitutions_insertions_and_deletions() {
        assert_eq!(word_error_rate("one two three", "one four"), 2.0 / 3.0);
        assert_eq!(word_error_rate("one two", "one two three"), 0.5);
        assert_eq!(word_error_rate("one two three", "one three"), 1.0 / 3.0);
    }

    #[test]
    fn wer_has_a_defined_empty_reference_behavior() {
        assert_eq!(word_error_rate("", ""), 0.0);
        assert!(word_error_rate("", "unexpected words").is_infinite());
    }

    #[test]
    fn alignment_diff_marks_matches_substitutions_deletions_and_insertions() {
        let diff = alignment_diff("one two three", "one four three extra");
        assert_eq!(diff, "  one\n- two\n+ four\n  three\n+ extra");
    }
}
