#![no_main]
//! Fuzz the input validator with arbitrary strings.
//!
//! Checks that validation never panics on malformed input.

use libfuzzer_sys::fuzz_target;
use thinclaw::safety::Validator;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let validator = Validator::new();
        // Should never panic on any input
        let _ = validator.validate(s);
    }
});
