//! F-09 drift guard for duplicated helper source across the workspace-excluded
//! `tools-src` crates.
//!
//! `url_encode_path` and `validate_input_length` (+ `MAX_TEXT_LENGTH`) are
//! currently byte-for-byte duplicated in `tools-src/github` and
//! `tools-src/notion`. The proper fix is to extract a shared `include!` source
//! module (WS-03 Wave-3 SDK packaging); until then this test fails CI if the
//! copies diverge, so the next fix (or security fix) cannot silently land in
//! only one copy — the exact pattern behind the earlier multibyte-UTF-8 panic.
//!
//! NOTE: the channel helpers (`split_message` / `byte_index_for_char_limit`)
//! are also duplicated but differ by doc/inline comments, so a raw comparison
//! is not reliable for them; their guard is deferred to the shared-module
//! extraction. This guard covers the byte-identical tools pair.

use std::path::Path;

/// Extract a top-level `fn <name>` body by capturing from the signature line to
/// the first column-0 `}` line. Mirrors the audit that confirmed these copies
/// are byte-identical; robust to `{`/`}` inside format strings (which never
/// appear as a lone `}` line).
fn extract_top_level_fn(src: &str, name: &str) -> String {
    let needle = format!("fn {name}(");
    let mut out = String::new();
    let mut capturing = false;
    for line in src.lines() {
        if !capturing && line.contains(&needle) {
            capturing = true;
        }
        if capturing {
            out.push_str(line);
            out.push('\n');
            if line == "}" {
                return out;
            }
        }
    }
    panic!("fn {name} not found (or no column-0 closing brace)");
}

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn github_and_notion_share_identical_helper_source() {
    let github = read_repo_file("tools-src/github/src/lib.rs");
    let notion = read_repo_file("tools-src/notion/src/lib.rs");

    for func in ["url_encode_path", "validate_input_length"] {
        assert_eq!(
            extract_top_level_fn(&github, func),
            extract_top_level_fn(&notion, func),
            "tools-src github/notion `{func}` diverged (F-09): re-sync the copies \
             or extract a shared `include!` helper so a fix cannot land in one only"
        );
    }

    // The shared 64 KiB input cap must also stay in lockstep.
    let cap_line = "const MAX_TEXT_LENGTH: usize = 65536;";
    assert!(
        github.contains(cap_line) && notion.contains(cap_line),
        "MAX_TEXT_LENGTH diverged between github/notion tools (F-09)"
    );
}
