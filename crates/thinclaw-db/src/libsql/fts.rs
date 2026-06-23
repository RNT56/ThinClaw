//! Shared FTS5 query sanitization for the libSQL backend.
//!
//! SQLite's FTS5 `MATCH` operator parses its right-hand side as a query
//! expression, not a literal string. That means raw user input can be
//! interpreted as FTS5 syntax: `-`, `:`, `"`, `*`, `^`, `(`, `)`, and the bare
//! keywords `AND`/`OR`/`NOT`/`NEAR` are all operators. A query like
//! `re-enable`, `time:sensitive`, or an unbalanced `"quote` is treated as
//! syntax and makes `MATCH` raise an error instead of searching for the text.
//!
//! Postgres tolerates the same input because `plainto_tsquery` /
//! `websearch_to_tsquery` have forgiving parsers; libSQL has no such tolerant
//! parser, so the input must be pre-sanitized to keep the two backends at
//! parity. The strategy here is the proven one already used by the workspace
//! memory search: tokenize on non-`[alphanumeric_]` characters and wrap each
//! surviving token in double quotes, so every token is matched as a literal
//! phrase and no character is interpreted as an operator. For example
//! `time-sensitive notes` becomes `"time" "sensitive" "notes"`.
//!
//! This is intentionally the *quote-each-token* core only. Higher-level recall
//! features such as morphological keyword expansion (`expand_query_keywords`)
//! are layered on top by individual call sites (see `workspace.rs`) so that
//! transcript search gains only the safety guarantee, not divergent ranking.

/// Sanitize raw user input for use as the right-hand side of an FTS5 `MATCH`.
///
/// Splits on any character that is not alphanumeric or `_`, drops empty
/// fragments, wraps each remaining token in double quotes, and joins them with
/// a single space. Returns an empty string when no tokens survive (e.g. the
/// input was empty, whitespace, or pure punctuation like `:::`).
///
/// Callers MUST treat an empty return value as "no searchable terms" and avoid
/// binding it to `MATCH` — `MATCH ''` is itself an FTS5 error. The convention
/// is to early-return an empty result set in that case.
pub(super) fn sanitize_fts5_match(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::sanitize_fts5_match;

    #[test]
    fn quotes_each_token() {
        assert_eq!(sanitize_fts5_match("hello world"), "\"hello\" \"world\"");
    }

    #[test]
    fn colon_is_split_not_an_operator() {
        // `foo:bar` would be a column filter in raw FTS5; quoting neutralizes it.
        assert_eq!(
            sanitize_fts5_match("time:sensitive"),
            "\"time\" \"sensitive\""
        );
        assert_eq!(sanitize_fts5_match("foo:bar"), "\"foo\" \"bar\"");
    }

    #[test]
    fn hyphen_is_split_not_a_negation() {
        // `re-enable` would parse as `re NOT enable` in raw FTS5.
        assert_eq!(sanitize_fts5_match("re-enable"), "\"re\" \"enable\"");
    }

    #[test]
    fn double_quote_does_not_leak_through() {
        // An unterminated quote would make raw FTS5 throw; we drop the quote
        // character entirely and only keep the inner token.
        assert_eq!(sanitize_fts5_match("\"unterminated"), "\"unterminated\"");
        assert_eq!(
            sanitize_fts5_match("\"quoted phrase\""),
            "\"quoted\" \"phrase\""
        );
    }

    #[test]
    fn underscore_is_preserved_inside_token() {
        assert_eq!(sanitize_fts5_match("hello_world"), "\"hello_world\"");
    }

    #[test]
    fn bare_boolean_keywords_are_quoted_as_literals() {
        // Quoting prevents AND/OR/NOT from being parsed as FTS5 operators.
        assert_eq!(
            sanitize_fts5_match("foo AND bar"),
            "\"foo\" \"AND\" \"bar\""
        );
    }

    #[test]
    fn empty_and_whitespace_yield_empty_string() {
        assert_eq!(sanitize_fts5_match(""), "");
        assert_eq!(sanitize_fts5_match("   "), "");
    }

    #[test]
    fn pure_punctuation_yields_empty_string() {
        // Caller must early-return rather than bind `MATCH ''`.
        assert_eq!(sanitize_fts5_match(":::"), "");
        assert_eq!(sanitize_fts5_match("-.-"), "");
    }

    #[test]
    fn mixed_punctuation_and_text() {
        assert_eq!(
            sanitize_fts5_match("re-enable the time:sensitive feature"),
            "\"re\" \"enable\" \"the\" \"time\" \"sensitive\" \"feature\""
        );
    }
}
