#[test]
fn canonical_reference_uses_current_root_and_removed_surface_contracts() {
    let reference = include_str!("../docs/CLI_REFERENCE.md");
    let surfaces = include_str!("../docs/SURFACES_AND_COMMANDS.md");
    for command in [
        "thinclaw setup",
        "thinclaw tui",
        "thinclaw ask",
        "thinclaw send",
        "thinclaw status",
    ] {
        assert!(
            reference.contains(command),
            "missing canonical command {command}"
        );
    }
    assert!(!surfaces.contains("!<command>"));
    assert!(!surfaces.contains("`/think` |"));
    assert!(surfaces.contains("/tools"));
}
