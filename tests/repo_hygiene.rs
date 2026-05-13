use std::process::Command;

const PACKAGE_ROOTS: &[&str] = &[
    "channels-src/",
    "tools-src/",
    "registry/channels/",
    "registry/tools/",
];

#[test]
fn package_source_trees_do_not_track_cargo_target_artifacts() {
    let output = Command::new("git")
        .args(["ls-files"])
        .output()
        .expect("git ls-files should run");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut violations: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| {
            PACKAGE_ROOTS.iter().any(|root| path.starts_with(root)) && path.contains("/target/")
        })
        .map(str::to_string)
        .collect();

    violations.sort();
    assert!(
        violations.is_empty(),
        "generated Cargo target artifacts must not be tracked in package source trees:\n{}",
        violations.join("\n")
    );
}

#[test]
fn package_source_trees_do_not_contain_generated_target_dirs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for package_root in PACKAGE_ROOTS {
        collect_target_dirs(&root.join(package_root), root, &mut violations);
    }

    violations.sort();
    assert!(
        violations.is_empty(),
        "generated Cargo target directories must not exist in package source trees; use CARGO_TARGET_DIR outside the repo:\n{}",
        violations.join("\n")
    );
}

fn collect_target_dirs(path: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if entry.file_name() == "target" {
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
            );
            continue;
        }
        collect_target_dirs(&path, root, out);
    }
}
