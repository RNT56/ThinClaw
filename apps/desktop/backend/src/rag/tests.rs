use super::{truncate_utf8_owned, valid_sha256_hex};

#[test]
fn document_hash_validation_is_exact() {
    assert!(valid_sha256_hex(&"a".repeat(64)));
    assert!(valid_sha256_hex(&"A0".repeat(32)));
    assert!(!valid_sha256_hex(&"a".repeat(63)));
    assert!(!valid_sha256_hex(&format!("{}g", "a".repeat(63))));
}

#[test]
fn utf8_truncation_preserves_character_boundaries() {
    let truncated = truncate_utf8_owned("🦀🦀🦀".to_string(), 9);
    assert_eq!(truncated, "🦀🦀");
    assert!(truncated.is_char_boundary(truncated.len()));
}
