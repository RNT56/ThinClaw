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
