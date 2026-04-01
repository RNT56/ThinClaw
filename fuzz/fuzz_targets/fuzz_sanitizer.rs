#![no_main]
//! Fuzz the content sanitizer with arbitrary byte strings.
//!
//! Checks that `Sanitizer::sanitize()` never panics, regardless of input.

use libfuzzer_sys::fuzz_target;
use thinclaw::safety::Sanitizer;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let sanitizer = Sanitizer::new();
        // Should never panic, regardless of input content
        let _ = sanitizer.sanitize(s);
    }
});
