#![no_main]
//! Fuzz credential pattern detection with arbitrary JSON values.
//!
//! Tests that the credential scanner handles all input
//! without panics or excessive runtime.

use libfuzzer_sys::fuzz_target;
use thinclaw::safety::params_contain_manual_credentials;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Try to parse as JSON; if it parses, scan for credentials
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            let _ = params_contain_manual_credentials(&value);
        }
    }
});
