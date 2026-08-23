use whisperspree_lib::testutil::wer::{normalize, word_error_rate};

#[test]
fn wer_normalizes_case_punctuation_whitespace_and_nfc() {
    assert_eq!(normalize(" Héllo,   WORLD! "), "héllo world");
}

#[test]
fn wer_is_word_level_levenshtein_over_reference() {
    assert_eq!(
        word_error_rate("one two three", "one four three"),
        1.0 / 3.0
    );
    assert_eq!(word_error_rate("anything", ""), 1.0);
}
