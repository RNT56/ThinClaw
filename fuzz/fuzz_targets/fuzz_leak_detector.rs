#![no_main]
//! Fuzz the leak detector with arbitrary content.
//!
//! Checks that leak detection never panics on arbitrary input patterns.

use libfuzzer_sys::fuzz_target;
use thinclaw::safety::LeakDetector;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let detector = LeakDetector::new();
        // scan + scan_and_clean should never panic
        let _ = detector.scan(s);
        let _ = detector.scan_and_clean(s);
    }
});
